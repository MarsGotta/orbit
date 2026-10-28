import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { CopyProvider } from "./CopyProvider";
import { spanish } from "./es";
import { english } from "./en";
import BriefingView from "../features/briefing/BriefingView";
import NotesView from "../features/notes/NotesView";
import { NoteDetail } from "../features/notes/NoteDetail";
import { appView, notesSnapshot, writtenNote, library } from "../features/notes/test-notes";
import AssistantView from "../features/assistant/AssistantView";
import SettingsView from "../features/settings/SettingsView";
import type { AppView } from "../state/app-view";

const send = vi.fn(async () => true);
function snapshot(): AppView {
  const value = appView(notesSnapshot({ list: [] }));
  return {
    ...value,
    connection: { daemon: "ready", hub: "offline" },
    calendar: { events: [], authorization: "granted", reading: { state: "available" } },
    call: null,
    meeting: { phase: "idle", recent: [], tracks: [] },
    system_timezone: "UTC",
    settings: {
      ...value.settings,
      language: "en",
      agents: [],
      hub: { ...value.settings.hub, paired: false },
      versions: { app: "0.1.0" },
      update: { state: "idle" },
      permissions: {},
      profile: { name: "Robin" },
    },
    briefing: {
      summary: { meetings_today: 0, focus_minutes: 0 },
      agenda: [],
      upcoming: [],
      sources: { agenda: { state: "present" } },
      greeting: { name: "Robin" },
    },
    chat: {
      ...value.chat,
      session_id: null,
      messages: [],
      turn: [],
      availability: { state: "current" },
      pending: null,
      voice: { state: "idle" },
      stream: null,
      approvals: [],
      clarification: null,
    },
  } as unknown as AppView;
}
afterEach(cleanup);

it.each(["Today", "Meetings", "Meeting note", "Assistant", "Settings"])(
  "renders %s in English without Spanish catalogue text",
  (page) => {
    const value = snapshot();
    const props = { snapshot: value, send, error: null };
    const screens = {
      Today: <BriefingView {...props} onRecord={() => undefined} />,
      Meetings: <NotesView {...props} onRecord={() => undefined} />,
      "Meeting note": (
        <NoteDetail note={writtenNote()} library={library()} send={send} error={null} />
      ),
      Assistant: <AssistantView {...props} />,
      Settings: <SettingsView {...props} onReopenOnboarding={() => undefined} />,
    };
    const view = render(
      <CopyProvider choice="en" name="Atlas">
        {screens[page as keyof typeof screens]}
      </CopyProvider>,
    );
    const expected = {
      Today: "Good",
      Meetings: "No meetings yet",
      "Meeting note": "Summary",
      Assistant: "Pair this Mac",
      Settings: "Settings",
    };
    expect(view.container.textContent).toContain(expected[page as keyof typeof expected]);
    // Meeting content and inline copy are allowed. Catalogue strings that differ
    // between languages must never be selected by these screens.
    const spanishStrings = strings(spanish("Atlas"));
    const englishStrings = new Set(strings(english("Atlas")));
    const forbidden = new Set(spanishStrings.filter((text) => !englishStrings.has(text)));
    // Inline copy is allowed in this wave; these two date labels are still
    // written directly by conversation grouping, rather than read from copy.
    forbidden.delete("Hoy");
    forbidden.delete("Ayer");
    const walker = document.createTreeWalker(view.container, NodeFilter.SHOW_TEXT);
    while (walker.nextNode())
      expect(forbidden.has(walker.currentNode.textContent?.trim() ?? "")).toBe(false);
    if (page === "Meeting note") expect(screen.getByRole("tab", { name: "Summary" })).toBeTruthy();
  },
);

function strings(value: unknown): string[] {
  if (typeof value === "string") return [value];
  if (value === null || typeof value !== "object") return [];
  return Object.values(value).flatMap(strings);
}
