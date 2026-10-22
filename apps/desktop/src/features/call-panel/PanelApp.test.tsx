import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import type { AppBootstrap } from "../../bridge/generated";
import type { DaemonPort } from "../../bridge/port";
import { PanelApp } from "./PanelApp";

afterEach(cleanup);

/** A port whose recording frame holds `meeting`; nothing else is said. */
function portWith(meeting: Record<string, unknown>): DaemonPort {
  const features = [
    "status",
    "local",
    "permissions",
    "agenda",
    "notes",
    "note",
    "recording",
    "assistant",
    "control",
    "briefing",
  ];
  return {
    subscribeFrames: async () => () => undefined,
    snapshot: async () =>
      ({
        revision: { epoch: 1, counter: 1 },
        frames: features.map((feature) => ({
          feature,
          revision: { epoch: 1, counter: 1 },
          view:
            feature === "recording"
              ? { meeting }
              : feature === "status"
                ? { settings: {}, notices: [], notice: null }
                : {},
        })),
      }) as unknown as AppBootstrap,
    report: async () => undefined,
    send: async () => ({ state: "accepted" }),
  } as unknown as DaemonPort;
}

const ask = { windows: [{ minutes: 2 }], default_minutes: 2, max_question_bytes: 1024 };

it("offers «Preguntar a Deimos» during a recording started by hand, with no call", async () => {
  render(
    <PanelApp
      port={portWith({
        phase: "recording",
        call_panel: null,
        ask,
        voice_trigger: { state: "listening" },
      })}
    />,
  );
  expect(await screen.findByRole("button", { name: "Preguntar a Deimos" })).toBeTruthy();
  expect(screen.getByText("«Oye Deimos» escucha en tu micrófono")).toBeTruthy();
});

it("offers nothing to ask outside a recording", async () => {
  const { container } = render(<PanelApp port={portWith({ phase: "idle", call_panel: null })} />);
  await waitFor(() => expect(container.querySelector("[data-panel-root]")).not.toBeNull());
  expect(screen.queryByRole("button", { name: "Preguntar a Deimos" })).toBeNull();
});
