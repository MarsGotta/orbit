import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import type { AppCommand } from "../bridge/generated";
import type { AppView as AppSnapshot } from "../state/app-view";
import type { ViewId } from "../state/ui";
import { Sidebar } from "./Sidebar";

const idle = {
  phase: "idle",
  recording_id: null,
  title: null,
  started_at_ms: null,
  ends_at_ms: null,
  limit_minutes: 240,
  audio: "system",
  tracks: [],
  recent: [],
} as unknown as AppSnapshot["meeting"];

function snapshot(change: Partial<AppSnapshot> = {}): AppSnapshot {
  return {
    chat: { pending: null },
    notes: { list: [] },
    settings: { profile: { name: "Alba Ríos" }, agents: [] },
    system_timezone: "Europe/Madrid",
    meeting: idle,
    call: null,
    ...change,
  } as unknown as AppSnapshot;
}

function renderSidebar(value = snapshot(), view: ViewId = "briefing") {
  const sent: AppCommand[] = [];
  let recorded = 0;
  render(
    <Sidebar
      onRecord={() => (recorded += 1)}
      send={(command) => sent.push(command)}
      snapshot={value}
      view={view}
    />,
  );
  return { sent, recorded: () => recorded };
}

describe("the sidebar", () => {
  it("names every section, as links, with the one open marked", () => {
    renderSidebar(snapshot(), "notes");
    const nav = screen.getByRole("navigation", { name: "Principal" });
    const links = within(nav).getAllByRole("link");
    expect(links.map((link) => [link.textContent, link.getAttribute("href")])).toEqual([
      ["Hoy", "#/briefing"],
      ["Calendario", "#/calendar"],
      ["Reuniones", "#/notes"],
      ["Asistente", "#/assistant"],
      ["Ajustes", "#/settings"],
    ]);
    expect(within(nav).getByRole("link", { name: "Reuniones" }).getAttribute("aria-current")).toBe(
      "page",
    );
    expect(within(nav).getByRole("link", { name: "Hoy" }).getAttribute("aria-current")).toBeNull();
    expect(nav.textContent).toContain("Alba Ríos");
  });

  it("«Grabar reunión» opens the new recording window and shows its shortcut", async () => {
    const { recorded } = renderSidebar();
    const button = screen.getByRole("button", { name: "Grabar reunión" });
    expect(button.textContent).toContain("⌃⌥M");
    await userEvent.click(button);
    expect(recorded()).toBe(1);
  });

  it("says how many meetings are on their way and when the assistant needs you", () => {
    renderSidebar(
      snapshot({
        notes: {
          list: [
            { recording_id: "a", title: "Uno", state: "transcribing" },
            { recording_id: "b", title: "Dos", state: "ready", headline: { phase: "ready" } },
            { recording_id: "c", title: "Tres", state: "failed" },
          ],
        } as unknown as AppSnapshot["notes"],
        chat: {
          pending: { kind: "approval", id: "p1", text: "Enviar un correo", choices: [] },
        } as unknown as AppSnapshot["chat"],
      }),
    );
    expect(screen.getByRole("link", { name: /Reuniones/ }).textContent).toBe("Reuniones1 en curso");
    expect(screen.getByRole("link", { name: /Asistente/ }).textContent).toContain("Te necesita");
  });

  it("while recording, the card takes the place of «Grabar reunión»", () => {
    renderSidebar(
      snapshot({
        meeting: { ...idle, phase: "recording", started_at_ms: Date.now(), title: "Comité" },
      }),
    );
    expect(screen.queryByRole("button", { name: "Grabar reunión" })).toBeNull();
    expect(screen.getByRole("region", { name: "Grabación en curso" })).toBeTruthy();
  });

  it("a call shows in any view, with its own card", () => {
    renderSidebar(
      snapshot({
        call: {
          started_at_ms: Date.now(),
          seeing_screen: false,
          last_line: null,
          speaking: false,
          expires_at_ms: Date.now() + 15 * 60_000,
          origin: "hotkey",
          idle_hangup_ms: 60_000,
        },
      }),
      "calendar",
    );
    expect(screen.getByRole("region", { name: "Llamada en curso" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Grabar reunión" })).toBeNull();
  });
});
