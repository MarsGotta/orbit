import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
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
      ["Notas", "#/vault"],
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

describe("the sidebar's logo", () => {
  afterEach(() => vi.unstubAllGlobals());

  const logo = () =>
    screen.getByRole("navigation", { name: "Principal" }).querySelector("svg[data-orbiting]");
  const working = (activity: string | null) =>
    snapshot({ chat: { pending: null, activity } as unknown as AppSnapshot["chat"] });
  const sidebar = (value: AppSnapshot) => (
    <Sidebar onRecord={() => undefined} send={() => true} snapshot={value} view="briefing" />
  );

  it("is the brand's small symbol at 20 px beside «Orbit»: the ring in the text's colour, Deimos in the accent", () => {
    renderSidebar();
    const mark = logo();
    expect(mark?.getAttribute("viewBox")).toBe("0 0 48 48");
    expect(mark?.getAttribute("width")).toBe("20");
    expect(mark?.querySelector("ellipse")).toBeNull();
    expect(
      Array.from(mark?.querySelectorAll("path") ?? []).map((path) => path.getAttribute("fill")),
    ).toEqual(["currentColor", "var(--o-accText)", "currentColor"]);
    expect(mark?.nextElementSibling?.textContent).toBe("Orbit");
  });

  it("turns Deimos while an assistant works on an answer, and stops when it is done", () => {
    const { rerender } = render(sidebar(working(null)));
    expect(logo()?.getAttribute("data-orbiting")).toBe("false");
    rerender(sidebar(working("thinking")));
    expect(logo()?.getAttribute("data-orbiting")).toBe("true");
    rerender(sidebar(working("responding")));
    expect(logo()?.getAttribute("data-orbiting")).toBe("true");
    // Transcribing the user's voice note is not the assistant at work.
    rerender(sidebar(working("transcribing")));
    expect(logo()?.getAttribute("data-orbiting")).toBe("false");
    rerender(sidebar(working(null)));
    expect(logo()?.getAttribute("data-orbiting")).toBe("false");
  });

  it("stays still with «Reducir movimiento»", () => {
    vi.stubGlobal("matchMedia", (query: string) => ({
      matches: query === "(prefers-reduced-motion: reduce)",
      media: query,
      addEventListener: () => undefined,
      removeEventListener: () => undefined,
    }));
    render(sidebar(working("thinking")));
    expect(logo()?.getAttribute("data-orbiting")).toBe("false");
  });

  /** A stand-in for the Web Animations API, which jsdom lacks: what was asked of it. */
  function animations(currentTime: number) {
    const calls: { keyframes: Keyframe[]; options: KeyframeAnimationOptions }[] = [];
    const animate = (keyframes: Keyframe[], options: KeyframeAnimationOptions) => {
      calls.push({ keyframes, options });
      return { currentTime, cancel: () => undefined } as unknown as Animation;
    };
    const prototype = Element.prototype as Element & { animate?: unknown };
    const before = prototype.animate;
    prototype.animate = animate;
    return {
      calls,
      restore: () => {
        prototype.animate = before;
      },
    };
  }

  it("turns once every 12 s, clockwise, and eases Deimos back the short way when the work ends", () => {
    // 9 s into a turn: 270°, so the way home is forwards, to 360°.
    const late = animations(9_000);
    try {
      const { rerender } = render(sidebar(working("thinking")));
      expect(late.calls[0]?.keyframes).toEqual([
        { transform: "rotate(0deg)" },
        { transform: "rotate(360deg)" },
      ]);
      expect(late.calls[0]?.options).toMatchObject({ duration: 12_000, iterations: Infinity });
      rerender(sidebar(working(null)));
      const back = late.calls[1];
      expect(back?.keyframes).toEqual([
        { transform: "rotate(270deg)" },
        { transform: "rotate(360deg)" },
      ]);
      expect(Number(back?.options.duration)).toBeLessThan(1_000);
    } finally {
      late.restore();
    }
    cleanup();
    // 3 s in: 90°, so it goes back, to 0°.
    const early = animations(3_000);
    try {
      const { rerender } = render(sidebar(working("responding")));
      rerender(sidebar(working(null)));
      expect(early.calls[1]?.keyframes).toEqual([
        { transform: "rotate(90deg)" },
        { transform: "rotate(0deg)" },
      ]);
    } finally {
      early.restore();
    }
  });

  it("with «Reducir movimiento» neither turns nor eases back", () => {
    const motion = animations(9_000);
    let reduce = false;
    vi.stubGlobal("matchMedia", (query: string) => ({
      matches: reduce && query === "(prefers-reduced-motion: reduce)",
      media: query,
      addEventListener: () => undefined,
      removeEventListener: () => undefined,
    }));
    try {
      const { rerender } = render(sidebar(working("thinking")));
      expect(motion.calls).toHaveLength(1);
      // Switched on while it turns: Deimos is home at once, without moving.
      reduce = true;
      rerender(sidebar(working("responding")));
      expect(logo()?.getAttribute("data-orbiting")).toBe("false");
      rerender(sidebar(working(null)));
      expect(motion.calls).toHaveLength(1);
    } finally {
      motion.restore();
    }
  });

  it("gives «Reuniones» the brand's bars", () => {
    renderSidebar();
    const link = screen.getByRole("link", { name: "Reuniones" });
    expect(link.querySelector("path")?.getAttribute("d")).toBe(
      "M4 10.5v3M8 7v10M12 4.5v15M16 7v10M20 10.5v3",
    );
  });
});
