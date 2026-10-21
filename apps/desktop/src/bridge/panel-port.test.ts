import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, expect, it } from "vitest";
import { PanelPort, panelFocus, panelOpenUrl } from "./panel-port";

const OP = "0123abcd-0000-4000-8000-000000000000";

afterEach(clearMocks);

/**
 * P9-11: the floating panel may only answer a call — record, not now, keep
 * recording, stop — through its own narrow order; everything else is
 * refused before it reaches Tauri.
 */
it("sends only the call answers, through the panel's own order", async () => {
  const calls: { command: string; payload: unknown }[] = [];
  mockIPC((command, payload) => {
    calls.push({ command, payload });
    return { revision: { epoch: 1, counter: 1 } };
  });
  const port = new PanelPort();
  await port.send({ command: "call_record", call_id: "call-1" }, OP);
  await port.send({ command: "call_dismiss", call_id: "call-1" }, OP);
  await port.send({ command: "recording_keep", recording_id: "recording-1" }, OP);
  await port.send({ command: "meeting_finish" }, OP);
  expect(calls.map((call) => call.command)).toEqual([
    "orbit_panel_send",
    "orbit_panel_send",
    "orbit_panel_send",
    "orbit_panel_send",
  ]);
  expect(calls[0]?.payload).toEqual({
    command: { command: "call_record", call_id: "call-1" },
    op: OP,
  });
  await expect(port.send({ command: "chat", text: "hola" }, OP)).rejects.toThrow();
  await expect(port.send({ command: "privacy_stop" }, OP)).rejects.toThrow();
  expect(calls).toHaveLength(4);
});

/**
 * P9-12: the panel also asks Deimos and closes the question, through the
 * same order, and takes the keyboard only while Mars writes.
 */
it("asks Deimos, closes the question and asks for the keyboard through its own order", async () => {
  const calls: { command: string; payload: unknown }[] = [];
  mockIPC((command, payload) => {
    calls.push({ command, payload });
    return { revision: { epoch: 1, counter: 1 } };
  });
  const port = new PanelPort();
  await port.send({ command: "meeting_ask", question: "¿Qué dijo Lucía?", minutes: 2 }, OP);
  await port.send({ command: "meeting_question_close", id: "turn-1" }, OP);
  await panelFocus(port, true);
  await panelFocus(port, false);
  expect(calls.map((call) => call.command)).toEqual([
    "orbit_panel_send",
    "orbit_panel_send",
    "orbit_panel_send",
    "orbit_panel_send",
  ]);
  expect(calls[2]?.payload).toEqual({ command: { command: "panel_focus", focus: true } });
  expect(calls[3]?.payload).toEqual({ command: { command: "panel_focus", focus: false } });
  await expect(
    port.send({ command: "meeting_start", title: "x", minutes: 1 }, OP),
  ).rejects.toThrow();
});

/** R3: an answer's link opens through the panel's own closed order. */
it("opens an answer's link through the panel's order", async () => {
  const calls: { command: string; payload: unknown }[] = [];
  mockIPC((command, payload) => {
    calls.push({ command, payload });
    return { revision: { epoch: 1, counter: 1 } };
  });
  await panelOpenUrl(new PanelPort(), "https://acme.example/acta");
  expect(calls).toEqual([
    {
      command: "orbit_panel_send",
      payload: { command: { command: "open_url", url: "https://acme.example/acta" } },
    },
  ]);
});
